//! The dock model behind Photoshop-style panel docking: every panel is a tab ([`PanelId`]) in a
//! panel group ([`Group`]); groups stack in the main dock column, in extra dock columns at the
//! left, right or bottom of the work area ([`Column`]), or in floating windows ([`Float`]). Drags
//! are [`Payload`]s dropped on [`DropTarget`]s ([`DockLayout::apply_drop`]).
//!
//! The six Photoshop Essentials groups keep their legacy identity (`Group::Color` …), their
//! visibility flags in `Panels` and their selected tab in `DockTabs`; groups made by tearing a tab
//! off are `Group::Custom(n)`. Everything here is pure data so tests, `ui.set {dock}` and
//! `ui.dock` drive it without a window.

use serde::{Deserialize, Serialize};

use crate::dock::{DockLayout, Group};
use crate::state::{DockTabs, UiState};

/// One panel (a tab of a panel group).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PanelId {
    Color,
    Swatches,
    Gradients,
    Patterns,
    Properties,
    Adjustments,
    Character,
    Paragraph,
    Navigator,
    Histogram,
    Info,
    History,
    Actions,
    LayerComps,
    Layers,
    Channels,
    Paths,
    /// Clip Studio Paint's Tool Sliders: brush Size and Opacity (see `tool_sliders`).
    ToolSliders,
    /// Clip Studio workspace: the current tool's sub-tools (see `sub_tool_panel`).
    SubTool,
    /// Clip Studio workspace: the selected sub-tool's settings.
    ToolProperty,
    /// Window › Extensions › CLIP STUDIO: materials downloaded in CLIP STUDIO (see `csp_library_ui`).
    ClipStudio,
    /// Clip Studio workspace: RGB / HLS / CMY sliders for the tool colours (see `color_slider_ui`).
    ColorSlider,
}

impl PanelId {
    pub const ALL: [PanelId; 22] = [
        PanelId::Color,
        PanelId::Swatches,
        PanelId::Gradients,
        PanelId::Patterns,
        PanelId::Properties,
        PanelId::Adjustments,
        PanelId::Character,
        PanelId::Paragraph,
        PanelId::Navigator,
        PanelId::Histogram,
        PanelId::Info,
        PanelId::History,
        PanelId::Actions,
        PanelId::LayerComps,
        PanelId::Layers,
        PanelId::Channels,
        PanelId::Paths,
        PanelId::ToolSliders,
        PanelId::SubTool,
        PanelId::ToolProperty,
        PanelId::ClipStudio,
        PanelId::ColorSlider,
    ];

    /// The tab label (English; translated where drawn).
    pub fn label(self) -> &'static str {
        match self {
            PanelId::Color => "Color",
            PanelId::Swatches => "Swatches",
            PanelId::Gradients => "Gradients",
            PanelId::Patterns => "Patterns",
            PanelId::Properties => "Properties",
            PanelId::Adjustments => "Adjustments",
            PanelId::Character => "Character",
            PanelId::Paragraph => "Paragraph",
            PanelId::Navigator => "Navigator",
            PanelId::Histogram => "Histogram",
            PanelId::Info => "Info",
            PanelId::History => "History",
            PanelId::Actions => "Actions",
            PanelId::LayerComps => "Layer Comps",
            PanelId::Layers => "Layers",
            PanelId::Channels => "Channels",
            PanelId::Paths => "Paths",
            PanelId::ToolSliders => "Tool Sliders",
            PanelId::SubTool => "Sub Tool",
            PanelId::ToolProperty => "Tool Property",
            PanelId::ClipStudio => "CLIP STUDIO",
            PanelId::ColorSlider => "Color Slider",
        }
    }

    /// The icon shown for the panel in a column collapsed to icons.
    pub fn icon(self) -> &'static str {
        match self {
            PanelId::Color => "palette",
            PanelId::Swatches => "swatch-book",
            PanelId::Gradients => "blend",
            PanelId::Patterns => "grid-3x3",
            PanelId::Properties => "settings",
            PanelId::Adjustments => "contrast",
            PanelId::Character => "type",
            PanelId::Paragraph => "align-left",
            PanelId::Navigator => "navigation",
            PanelId::Histogram => "gauge",
            PanelId::Info => "info",
            PanelId::History => "clock",
            PanelId::Actions => "play",
            PanelId::LayerComps => "copy",
            PanelId::Layers => "layers",
            PanelId::Channels => "panels-top-left",
            PanelId::Paths => "spline",
            PanelId::ToolSliders => "sliders-horizontal",
            PanelId::SubTool => "brush",
            PanelId::ToolProperty => "settings",
            PanelId::ClipStudio => "puzzle",
            PanelId::ColorSlider => "sliders-horizontal",
        }
    }

    /// The serialised name, also the `window.panel.<key>` menu id suffix.
    pub fn key(self) -> &'static str {
        match self {
            PanelId::Color => "color",
            PanelId::Swatches => "swatches",
            PanelId::Gradients => "gradients",
            PanelId::Patterns => "patterns",
            PanelId::Properties => "properties",
            PanelId::Adjustments => "adjustments",
            PanelId::Character => "character",
            PanelId::Paragraph => "paragraph",
            PanelId::Navigator => "navigator",
            PanelId::Histogram => "histogram",
            PanelId::Info => "info",
            PanelId::History => "history",
            PanelId::Actions => "actions",
            PanelId::LayerComps => "layerComps",
            PanelId::Layers => "layers",
            PanelId::Channels => "channels",
            PanelId::Paths => "paths",
            PanelId::ToolSliders => "toolSliders",
            PanelId::SubTool => "subTool",
            PanelId::ToolProperty => "toolProperty",
            PanelId::ClipStudio => "clipStudio",
            PanelId::ColorSlider => "colorSlider",
        }
    }

    pub fn from_key(key: &str) -> Option<PanelId> {
        PanelId::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The Essentials group the panel belongs to by default (Tool Sliders has none: it floats).
    pub fn home(self) -> Option<Group> {
        Some(match self {
            PanelId::Color | PanelId::Swatches | PanelId::Gradients | PanelId::Patterns => Group::Color,
            PanelId::Properties | PanelId::Adjustments => Group::Properties,
            PanelId::Character | PanelId::Paragraph => Group::Character,
            PanelId::Navigator | PanelId::Histogram | PanelId::Info => Group::Navigator,
            PanelId::History | PanelId::Actions | PanelId::LayerComps => Group::History,
            PanelId::Layers | PanelId::Channels | PanelId::Paths => Group::Layers,
            PanelId::ToolSliders | PanelId::SubTool | PanelId::ToolProperty | PanelId::ClipStudio | PanelId::ColorSlider => return None,
        })
    }

    /// Panels that lay out their own list and footer, so they fill the group instead of scrolling.
    pub fn fills(self) -> bool {
        matches!(self, PanelId::Layers | PanelId::History | PanelId::Actions | PanelId::ToolSliders | PanelId::ClipStudio)
    }

    /// Size of the floating window a panel without a home group opens in.
    pub fn float_size(self) -> [f32; 2] {
        match self {
            PanelId::ClipStudio => [380.0, 560.0],
            PanelId::ColorSlider => [300.0, 170.0],
            _ => [180.0, 420.0],
        }
    }
}

/// The edge of the work area an extra dock column sits at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Left,
    #[default]
    Right,
    /// A row above the status bar; its groups sit side by side.
    Bottom,
}

impl Side {
    /// Width (left/right) or height (bottom) a new column gets.
    pub fn default_size(self) -> f32 {
        match self {
            Side::Bottom => 220.0,
            _ => 260.0,
        }
    }

    pub fn size_range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            Side::Bottom => 90.0..=700.0,
            _ => 160.0..=620.0,
        }
    }
}

/// An extra dock column (or, at the bottom, a row) of panel groups.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Column {
    /// Stable id (egui ids and `ui.dock`).
    pub id: u32,
    pub side: Side,
    /// Top to bottom (left to right at the bottom).
    pub groups: Vec<Group>,
    /// Width, or height for a bottom row (points).
    pub size: f32,
    /// Collapsed to icons («).
    pub iconic: bool,
}

/// A floating panel window: groups stacked top to bottom under a small title bar.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Float {
    pub id: u32,
    pub groups: Vec<Group>,
    /// Top-left corner (points, window coordinates).
    pub pos: [f32; 2],
    /// Width and height (points), title bar and resize grip included.
    pub size: [f32; 2],
    /// Collapsed to icons («).
    pub iconic: bool,
}

/// Bounds on a floating window's size.
pub const FLOAT_W: std::ops::RangeInclusive<f32> = 160.0..=1400.0;
pub const FLOAT_H: std::ops::RangeInclusive<f32> = 60.0..=2400.0;

/// What a dock drag carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Payload {
    /// One tab, torn off its group.
    Tab { group: Group, panel: PanelId },
    /// A whole group (dragged by its tab strip).
    Group(Group),
    /// A floating window (dragged by its title bar).
    Float(u32),
    /// A panel from outside the dock (Tool Sliders' own window).
    Panel(PanelId),
}

/// Where a drop puts the payload.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DropTarget {
    /// Into `group`'s tab strip, as tab `index`.
    Tabs { group: Group, index: usize },
    /// A group of its own just before (or `after`) `anchor`, in `anchor`'s column or window.
    Stack { anchor: Group, after: bool },
    /// A new dock column at `side`; `index` 0 is the outermost one.
    Column { side: Side, index: usize },
    /// A new floating window with its top-left corner at `pos`.
    Float { pos: [f32; 2] },
}

/// What kind of thing is dragged, for [`allowed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    /// A tab (or Tool Sliders from its own window).
    Panel,
    /// A whole group or floating window.
    Group,
    /// The Tools panel (`tools_window`): it docks beside other columns but never joins a group,
    /// never takes tabs and never docks at the top or bottom.
    Tools,
}

impl Payload {
    pub fn kind(self) -> DragKind {
        match self {
            Payload::Tab { .. } | Payload::Panel(_) => DragKind::Panel,
            Payload::Group(_) | Payload::Float(_) => DragKind::Group,
        }
    }
}

/// Can `kind` be dropped on `target`?
pub fn allowed(kind: DragKind, target: &DropTarget) -> bool {
    match kind {
        DragKind::Panel | DragKind::Group => true,
        DragKind::Tools => matches!(target, DropTarget::Column { side: Side::Left | Side::Right, .. }),
    }
}

/// A container of groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Main,
    Column(usize),
    Float(usize),
}

/// Most groups, columns and windows a layout keeps (guards `ui.set` and `ui.dock`).
const MAX_ITEMS: usize = 64;

impl DockLayout {
    /// Is `g` in an extra column or a floating window?
    pub fn placed_elsewhere(&self, g: Group) -> bool {
        self.columns.iter().any(|c| c.groups.contains(&g)) || self.floats.iter().any(|f| f.groups.contains(&g))
    }

    /// The container holding `g`, and its position there.
    pub fn slot_of(&self, g: Group) -> Option<(Slot, usize)> {
        if let Some((i, at)) = self.columns.iter().enumerate().find_map(|(i, c)| c.groups.iter().position(|x| *x == g).map(|at| (i, at))) {
            return Some((Slot::Column(i), at));
        }
        if let Some((i, at)) = self.floats.iter().enumerate().find_map(|(i, f)| f.groups.iter().position(|x| *x == g).map(|at| (i, at))) {
            return Some((Slot::Float(i), at));
        }
        self.order().iter().position(|x| *x == g).map(|at| (Slot::Main, at))
    }

    /// The groups of a container, in order.
    pub fn groups_in(&self, slot: Slot) -> Vec<Group> {
        match slot {
            Slot::Main => self.order(),
            Slot::Column(i) => self.columns.get(i).map(|c| c.groups.clone()).unwrap_or_default(),
            Slot::Float(i) => self.floats.get(i).map(|f| f.groups.clone()).unwrap_or_default(),
        }
    }

    fn groups_in_mut(&mut self, slot: Slot) -> Option<&mut Vec<Group>> {
        match slot {
            Slot::Main => {
                self.order = self.order();
                Some(&mut self.order)
            }
            Slot::Column(i) => self.columns.get_mut(i).map(|c| &mut c.groups),
            Slot::Float(i) => self.floats.get_mut(i).map(|f| &mut f.groups),
        }
    }

    /// Every group the layout knows: the Essentials groups and the custom ones.
    pub fn all_groups(&self) -> Vec<Group> {
        let mut out: Vec<Group> = Group::ALL.to_vec();
        out.extend(self.tabs.keys().copied().filter(|g| matches!(g, Group::Custom(_))));
        out
    }

    /// The tabs of `g`, in order. An Essentials group without stored tabs has its default ones,
    /// less any that were moved into another group.
    pub fn tabs_of(&self, g: Group, pro: bool) -> Vec<PanelId> {
        if let Some(t) = self.tabs.get(&g) {
            return t.clone();
        }
        let moved = |p: &PanelId| self.tabs.values().any(|t| t.contains(p));
        g.default_panels(pro).into_iter().filter(|p| !moved(p)).collect()
    }

    pub fn set_tabs(&mut self, g: Group, mut tabs: Vec<PanelId>) {
        let mut seen = Vec::with_capacity(tabs.len());
        tabs.retain(|p| {
            let fresh = !seen.contains(p);
            seen.push(*p);
            fresh
        });
        self.tabs.insert(g, tabs);
    }

    /// The group showing `panel`, if any.
    pub fn find_panel(&self, panel: PanelId, pro: bool) -> Option<Group> {
        self.all_groups().into_iter().find(|g| self.tabs_of(*g, pro).contains(&panel))
    }

    /// The selected tab of `g` (its tabs are `tabs`): an Essentials group's `DockTabs` index
    /// names one of its default tabs; a tab moved in from elsewhere is remembered in `active`.
    pub fn active_panel(&self, g: Group, tabs: &[PanelId], dock_tabs: &DockTabs, pro: bool) -> Option<PanelId> {
        if let Some(p) = self.active.get(&g).filter(|p| tabs.contains(p)) {
            return Some(*p);
        }
        let legacy = g.tab_index(dock_tabs).and_then(|i| g.default_panels(pro).get(i).copied());
        legacy.filter(|p| tabs.contains(p)).or_else(|| tabs.first().copied())
    }

    /// Select `panel` in `g`.
    pub fn set_active(&mut self, g: Group, panel: PanelId, dock_tabs: &mut DockTabs, pro: bool) {
        let legacy = g.default_panels(pro).iter().position(|p| *p == panel);
        match (g.tab_index_mut(dock_tabs), legacy) {
            (Some(slot), Some(i)) => {
                *slot = i;
                self.active.remove(&g);
            }
            _ => {
                self.active.insert(g, panel);
            }
        }
    }

    /// A new, empty custom group id.
    /// A new dock column at the outer edge of `side` holding one group per entry of `groups`
    /// (panels already docked elsewhere move into it). Returns the column id.
    pub(crate) fn add_column(&mut self, side: Side, groups: Vec<Vec<PanelId>>, size: f32) -> u32 {
        for tabs in &groups {
            for panel in tabs {
                for t in self.tabs.values_mut() {
                    t.retain(|p| p != panel);
                }
            }
        }
        let groups: Vec<Group> = groups.into_iter().map(|tabs| self.new_group(tabs)).collect();
        let id = self.new_id();
        self.columns.insert(0, Column { id, side, groups, size, iconic: false });
        id
    }

    fn new_group(&mut self, tabs: Vec<PanelId>) -> Group {
        let mut n = self.next_id.max(1);
        while self.tabs.contains_key(&Group::Custom(n)) {
            n = n.saturating_add(1);
        }
        self.next_id = n.saturating_add(1);
        let g = Group::Custom(n);
        self.set_tabs(g, tabs);
        g
    }

    fn new_id(&mut self) -> u32 {
        let mut n = self.next_id.max(1);
        while self.columns.iter().any(|c| c.id == n) || self.floats.iter().any(|f| f.id == n) || self.tabs.contains_key(&Group::Custom(n)) {
            n = n.saturating_add(1);
        }
        self.next_id = n.saturating_add(1);
        n
    }

    /// Take `g` out of whatever holds it.
    fn detach_group(&mut self, g: Group) {
        self.order = self.order();
        self.order.retain(|x| *x != g);
        for c in &mut self.columns {
            c.groups.retain(|x| *x != g);
        }
        for f in &mut self.floats {
            f.groups.retain(|x| *x != g);
        }
    }

    /// Drop custom groups without tabs, Essentials groups emptied out of extra columns and
    /// windows (they go back to the main column, empty and so not drawn), and empty columns and
    /// windows.
    pub fn cleanup(&mut self) {
        let dead: Vec<Group> = self.tabs.iter().filter(|(g, t)| matches!(g, Group::Custom(_)) && t.is_empty()).map(|(g, _)| *g).collect();
        for g in &dead {
            self.tabs.remove(g);
            self.active.remove(g);
            self.heights.remove(g);
            self.collapsed.retain(|x| x != g);
            self.hidden.retain(|x| x != g);
        }
        // Tab membership doesn't depend on the theme (only the default order does).
        let live: Vec<Group> = self.all_groups().into_iter().filter(|g| !self.tabs_of(*g, true).is_empty()).collect();
        let tabs = self.tabs.clone();
        let mut seen: Vec<Group> = Vec::new();
        for c in &mut self.columns {
            c.groups.retain(|g| {
                let keep = live.contains(g) && !seen.contains(g);
                seen.push(*g);
                keep
            });
        }
        for f in &mut self.floats {
            f.groups.retain(|g| {
                let keep = live.contains(g) && !seen.contains(g);
                seen.push(*g);
                keep
            });
        }
        self.columns.retain(|c| !c.groups.is_empty());
        self.floats.retain(|f| !f.groups.is_empty());
        self.columns.truncate(MAX_ITEMS);
        self.floats.truncate(MAX_ITEMS);
        self.order.retain(|g| !matches!(g, Group::Custom(_)) || tabs.get(g).is_some_and(|t| !t.is_empty()));
        let order = self.order();
        self.order = order;
        self.active.retain(|g, p| tabs.get(g).is_none_or(|t| t.contains(p)));
    }

    /// Apply a drop. `Ok(false)` when it changes nothing (a group dropped on itself); an error
    /// when the payload or target names something the layout doesn't have.
    pub fn apply_drop(&mut self, payload: Payload, target: DropTarget, dock_tabs: &mut DockTabs, pro: bool) -> Result<bool, String> {
        let exists = |l: &DockLayout, g: Group| !matches!(g, Group::Custom(_)) || l.tabs.contains_key(&g);
        match target {
            DropTarget::Tabs { group, .. } if !exists(self, group) => return Err(format!("no group {}", group.key())),
            DropTarget::Stack { anchor, .. } if !exists(self, anchor) => return Err(format!("no group {}", anchor.key())),
            DropTarget::Float { pos } if !pos.iter().all(|v| v.is_finite()) => return Err("float position must be finite".into()),
            _ => {}
        }
        if self.columns.len() >= MAX_ITEMS || self.floats.len() >= MAX_ITEMS || self.tabs.len() >= MAX_ITEMS * 2 {
            return Err("too many dock groups".into());
        }
        // Resolve a panel from outside the dock that is in fact in a group.
        let payload = match payload {
            Payload::Panel(p) => match self.find_panel(p, pro) {
                Some(group) => Payload::Tab { group, panel: p },
                None => Payload::Panel(p),
            },
            other => other,
        };
        // Drops that leave everything where it is.
        match (payload, target) {
            (Payload::Group(g), DropTarget::Tabs { group, .. } | DropTarget::Stack { anchor: group, .. }) if g == group => return Ok(false),
            (Payload::Float(id), DropTarget::Tabs { group, .. } | DropTarget::Stack { anchor: group, .. })
                if self.floats.iter().any(|f| f.id == id && f.groups.contains(&group)) =>
            {
                return Ok(false);
            }
            (Payload::Tab { group, .. }, DropTarget::Stack { anchor, .. }) if group == anchor && self.tabs_of(group, pro).len() <= 1 => return Ok(false),
            _ => {}
        }
        enum Moving {
            Panels(Vec<PanelId>),
            Groups(Vec<Group>, Option<[f32; 2]>),
        }
        let into_tabs = matches!(target, DropTarget::Tabs { .. });
        let moving = match payload {
            Payload::Tab { group, panel } => {
                let mut tabs = self.tabs_of(group, pro);
                let Some(from) = tabs.iter().position(|p| *p == panel) else {
                    return Err(format!("{} is not in group {}", panel.key(), group.key()));
                };
                if let DropTarget::Tabs { group: to, index } = target
                    && to == group
                {
                    // Reorder within the strip.
                    tabs.remove(from);
                    let at = if index > from { index - 1 } else { index }.min(tabs.len());
                    tabs.insert(at, panel);
                    let changed = at != from;
                    self.set_tabs(group, tabs);
                    self.set_active(group, panel, dock_tabs, pro);
                    return Ok(changed);
                }
                tabs.remove(from);
                self.set_tabs(group, tabs);
                Moving::Panels(vec![panel])
            }
            Payload::Panel(p) => Moving::Panels(vec![p]),
            Payload::Group(g) => {
                if !exists(self, g) {
                    return Err(format!("no group {}", g.key()));
                }
                if into_tabs {
                    let tabs = self.tabs_of(g, pro);
                    self.set_tabs(g, Vec::new());
                    Moving::Panels(tabs)
                } else {
                    self.detach_group(g);
                    Moving::Groups(vec![g], None)
                }
            }
            Payload::Float(id) => {
                let Some(i) = self.floats.iter().position(|f| f.id == id) else { return Err(format!("no floating window {id}")) };
                let f = self.floats.remove(i);
                if into_tabs {
                    let mut panels = Vec::new();
                    for g in &f.groups {
                        panels.extend(self.tabs_of(*g, pro));
                        self.set_tabs(*g, Vec::new());
                        self.order = self.order();
                    }
                    Moving::Panels(panels)
                } else {
                    Moving::Groups(f.groups, Some(f.size))
                }
            }
        };
        let (groups, size) = match moving {
            Moving::Panels(panels) => {
                if let DropTarget::Tabs { group, index } = target {
                    let mut tabs = self.tabs_of(group, pro);
                    let at = index.min(tabs.len());
                    for (k, p) in panels.iter().enumerate() {
                        tabs.insert((at + k).min(tabs.len()), *p);
                    }
                    self.set_tabs(group, tabs);
                    if let Some(p) = panels.first() {
                        self.set_active(group, *p, dock_tabs, pro);
                    }
                    self.hidden.retain(|x| *x != group);
                    self.cleanup();
                    return Ok(true);
                }
                (vec![self.new_group(panels)], None)
            }
            Moving::Groups(groups, size) => (groups, size),
        };
        match target {
            DropTarget::Tabs { .. } => {}
            DropTarget::Stack { anchor, after } => {
                let (slot, at) = self.slot_of(anchor).unwrap_or((Slot::Main, usize::MAX));
                // A floating window grows to make room for what is stacked into it.
                let extra: f32 = groups.iter().map(|g| self.height(*g) + crate::dock::GAP).sum();
                if let Slot::Float(i) = slot
                    && let Some(f) = self.floats.get_mut(i)
                {
                    f.size[1] = (f.size[1] + extra).clamp(*FLOAT_H.start(), *FLOAT_H.end());
                }
                if let Some(list) = self.groups_in_mut(slot) {
                    let at = if at == usize::MAX { list.len() } else { at + usize::from(after) }.min(list.len());
                    for (k, g) in groups.iter().enumerate() {
                        list.insert((at + k).min(list.len()), *g);
                    }
                }
            }
            DropTarget::Column { side, index } => {
                let id = self.new_id();
                let col = Column { id, side, groups, size: side.default_size(), iconic: false };
                let at = self.columns.iter().enumerate().filter(|(_, c)| c.side == side).nth(index).map_or(self.columns.len(), |(i, _)| i);
                self.columns.insert(at, col);
            }
            DropTarget::Float { pos } => {
                let id = self.new_id();
                let h = 14.0 + 8.0 + groups.iter().map(|g| self.height(*g)).sum::<f32>();
                let size = size.unwrap_or([280.0, h.clamp(*FLOAT_H.start(), 700.0)]);
                self.floats.push(Float { id, groups, pos, size, iconic: false });
            }
        }
        self.cleanup();
        Ok(true)
    }
}

/// Is group `g` shown? Essentials groups follow their `Panels` flag (Studio floats Properties
/// outside the dock); custom groups are shown unless closed.
pub fn group_visible(ui: &UiState, g: Group, pro: bool) -> bool {
    match g {
        Group::Custom(_) => !ui.dock.hidden.contains(&g),
        Group::Properties if !pro => false,
        _ => g.shown(&ui.panels),
    }
}

/// The groups of `slot` that are drawn: shown and with at least one tab.
pub fn visible_groups(ui: &UiState, slot: Slot, pro: bool) -> Vec<Group> {
    ui.dock.groups_in(slot).into_iter().filter(|g| group_visible(ui, *g, pro) && !ui.dock.tabs_of(*g, pro).is_empty()).collect()
}

/// Show or hide group `g` (its panels stay where they are).
pub fn set_group_visible(ui: &mut UiState, g: Group, on: bool) {
    match g {
        Group::Custom(_) => {
            ui.dock.hidden.retain(|x| *x != g);
            if !on {
                ui.dock.hidden.push(g);
            }
        }
        _ => *g.shown_mut(&mut ui.panels) = on,
    }
}

/// Is `panel` showing: in a shown, expanded group as its selected tab?
pub fn panel_showing(ui: &UiState, panel: PanelId, pro: bool) -> bool {
    let Some(g) = ui.dock.find_panel(panel, pro) else { return false };
    let tabs = ui.dock.tabs_of(g, pro);
    group_visible(ui, g, pro) && !ui.dock.is_collapsed(g) && ui.dock.active_panel(g, &tabs, &ui.dock_tabs, pro) == Some(panel)
}

/// The Window menu checkmark: `panel` is the selected tab of a shown group (collapsed or not).
pub fn panel_checked(ui: &UiState, panel: PanelId, pro: bool) -> bool {
    let Some(g) = ui.dock.find_panel(panel, pro) else { return false };
    let tabs = ui.dock.tabs_of(g, pro);
    group_visible(ui, g, pro) && ui.dock.active_panel(g, &tabs, &ui.dock_tabs, pro) == Some(panel)
}

/// Window › <panel>: a showing panel's group is hidden (as in Photoshop); otherwise the panel is
/// brought back wherever it is — an Essentials panel closed out of every group returns to its
/// home group, Tool Sliders to a floating window of its own — and selected. `show` forces one
/// way. Returns whether the panel is now showing.
pub fn toggle_panel(ui: &mut UiState, panel: PanelId, pro: bool, show: Option<bool>) -> bool {
    let showing = panel_showing(ui, panel, pro);
    if !show.unwrap_or(!showing) {
        if let Some(g) = ui.dock.find_panel(panel, pro).filter(|_| showing || show == Some(false)) {
            set_group_visible(ui, g, false);
        }
        return false;
    }
    let g = match ui.dock.find_panel(panel, pro) {
        Some(g) => g,
        None => match panel.home() {
            Some(home) => {
                let mut tabs = ui.dock.tabs_of(home, pro);
                let defaults = home.default_panels(pro);
                let rank = |p: &PanelId| defaults.iter().position(|d| d == p).unwrap_or(usize::MAX);
                let at = tabs.iter().position(|p| rank(p) > rank(&panel)).unwrap_or(tabs.len());
                tabs.insert(at, panel);
                ui.dock.set_tabs(home, tabs);
                home
            }
            None => {
                let g = ui.dock.new_group(vec![panel]);
                let id = ui.dock.new_id();
                ui.dock.floats.push(Float { id, groups: vec![g], pos: [320.0, 160.0], size: panel.float_size(), iconic: false });
                g
            }
        },
    };
    set_group_visible(ui, g, true);
    if panel == PanelId::ToolSliders {
        ui.panels.tool_sliders = true;
    }
    ui.dock.set_collapsed(g, false);
    let mut dock_tabs = ui.dock_tabs;
    ui.dock.set_active(g, panel, &mut dock_tabs, pro);
    ui.dock_tabs = dock_tabs;
    true
}

/// Close one tab: it leaves its group (Window › <panel> brings it back to its home group).
pub fn close_panel(ui: &mut UiState, g: Group, panel: PanelId, pro: bool) {
    let mut tabs = ui.dock.tabs_of(g, pro);
    tabs.retain(|p| *p != panel);
    let empty = tabs.is_empty();
    ui.dock.set_tabs(g, tabs);
    if panel == PanelId::ToolSliders {
        ui.panels.tool_sliders = false;
    }
    if empty && !matches!(g, Group::Custom(_)) {
        // An emptied Essentials group closes (its empty tab list stays, so its other default
        // tabs don't come back by themselves); Window › <panel> refills it.
        set_group_visible(ui, g, false);
    }
    ui.dock.cleanup();
}

/// Is Tool Sliders a tab in a dock group (so its own window and docked panel stay away)?
pub fn sliders_grouped(ui: &UiState) -> bool {
    ui.dock.tabs.values().any(|t| t.contains(&PanelId::ToolSliders))
}

/// Per-frame upkeep before the dock is drawn: Tool Sliders closed from the Window menu (or its
/// own panel menu) leaves the group it was in.
pub fn sync(ui: &mut UiState) {
    if !ui.panels.tool_sliders && sliders_grouped(ui) {
        for t in ui.dock.tabs.values_mut() {
            t.retain(|p| *p != PanelId::ToolSliders);
        }
        ui.dock.cleanup();
    }
}

#[cfg(test)]
#[path = "dock_model_tests.rs"]
mod tests;
